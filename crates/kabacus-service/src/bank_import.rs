// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::books::{with_tags, WorkingCopy};
use crate::caller::{Caller, Permission};
use crate::error::Error;
use crate::service::{invalid, require};
use crate::{ops, search, Service};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::NaiveDate;
use kabacus_core::bank_import::{
    self as core, CsvMapping, Decision, ImportError, Outcome, PlanInput, SimplefinFeed, Statement,
};
use kabacus_core::funds::UNCATEGORIZED;
use kabacus_core::{
    AccountKind, AccountName, Amount, Balances, Posting, PostingKind, Status, Transaction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_rows: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 5 * 1024 * 1024,
            max_rows: 5000,
        }
    }
}

impl Limits {
    pub fn max_request_bytes(self) -> usize {
        self.max_bytes.saturating_mul(6).saturating_add(1024 * 1024)
    }
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ImportOptions {
    #[serde(skip)]
    pub(crate) require_unambiguous: bool,
    /// Defaults to true. Save by sending false with the previewed head.
    pub dry_run: Option<bool>,
    pub since: Option<NaiveDate>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
    #[serde(default)]
    pub include_duplicates: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FileFormat {
    Ofx,
    Qfx,
    Csv,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ImportBody {
    pub format: FileFormat,
    pub content: Option<String>,
    pub content_base64: Option<String>,
    pub filename: Option<String>,
    /// Index in the list returned by a preview of an OFX file with several statements.
    pub statement: Option<usize>,
    /// CSV column indices are zero-based. A preview without a mapping inspects the file.
    pub mapping: Option<CsvMapping>,
    #[serde(default)]
    pub skip_rows: usize,
    #[serde(flatten)]
    pub options: ImportOptions,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ImportParams {
    pub account: String,
    pub head: String,
    #[serde(flatten)]
    pub body: ImportBody,
}

impl From<ImportError> for Error {
    fn from(error: ImportError) -> Self {
        let code = match &error {
            ImportError::TooLarge => "import_too_large",
            ImportError::AccountKind(_) => "account_kind",
            ImportError::CurrencyMismatch { .. } => "currency_mismatch",
            ImportError::DateFormat => "date_format",
            ImportError::Field { field, .. }
                if matches!(field.as_str(), "amount" | "balance" | "TRNAMT" | "BALAMT") =>
            {
                "amount"
            }
            ImportError::Funds(kabacus_core::FundsError::NoReserve { .. }) => "missing_reserve",
            _ => "statement_import",
        };
        Error::Import {
            code,
            message: error.to_string(),
        }
    }
}

fn file_content(body: &mut ImportBody, limits: Limits) -> Result<Vec<u8>, Error> {
    let bytes = match (body.content.take(), body.content_base64.take()) {
        (Some(text), None) => text.into_bytes(),
        (None, Some(encoded)) => {
            if encoded.len() > limits.max_bytes.div_ceil(3).saturating_mul(4) {
                return Err(ImportError::TooLarge.into());
            }
            STANDARD.decode(encoded).map_err(|_| {
                invalid(
                    "content_base64",
                    "Enter valid Base64 statement content".into(),
                )
            })?
        }
        _ => {
            return Err(invalid(
                "content",
                "Supply exactly one of content or content_base64".into(),
            ))
        }
    };
    if bytes.len() > limits.max_bytes {
        return Err(ImportError::TooLarge.into());
    }
    Ok(bytes)
}

pub async fn import_statement(
    state: &Service,
    caller: &dyn Caller,
    target: &str,
    head: &str,
    mut body: ImportBody,
) -> Result<Value, Error> {
    require(caller, Permission::Write)?;
    if head.is_empty() {
        return Err(invalid("head", "The current head is required".into()));
    }
    let mut wc = WorkingCopy::load(state, head).await?;
    let limits = state.settings.statement_limits;
    let bytes = file_content(&mut body, limits)?;
    let commodity = wc.commodity();
    let places = wc.places(&commodity);
    let dry_run = body.options.dry_run.unwrap_or(true);
    let statement = match body.format {
        FileFormat::Csv => {
            let Some(mapping) = &body.mapping else {
                if !dry_run {
                    return Err(ImportError::CsvMapping.into());
                }
                return Ok(json!({"head": wc.head, "dry_run": true, "account": target,
                    "source": {"format": "csv"}, "csv": core::inspect_csv(&bytes, body.skip_rows)?}));
            };
            core::parse_csv(&bytes, mapping, places, limits.max_rows)?
        }
        FileFormat::Ofx | FileFormat::Qfx => {
            let mut statements = core::parse_ofx(&bytes, u32::MAX, limits.max_rows)?;
            if let Some(index) = body.statement {
                if index >= statements.len() {
                    return Err(invalid(
                        "statement",
                        "Choose a statement from the preview".into(),
                    ));
                }
                statements.swap_remove(index)
            } else if statements.len() == 1 {
                statements.remove(0)
            } else if dry_run {
                let choices: Vec<_> = statements.into_iter().enumerate().map(|(index, statement)| {
                    json!({"statement": index, "account_hint": statement.account_hint,
                        "kind": statement.kind.map(AccountKind::as_str), "rows": statement.rows.len()})
                }).collect();
                return Ok(json!({"head": wc.head, "dry_run": true, "account": target,
                    "source": {"format": "ofx"}, "statements": choices}));
            } else {
                return Err(invalid(
                    "statement",
                    "Choose a statement from the preview".into(),
                ));
            }
        }
    };
    let mut result = apply_statement(&mut wc, target, statement, &body.options, None)?;
    ops::check(&wc)?;
    let changes = wc.changes();
    if !dry_run && !changes.is_empty() {
        let added = result["counts"]["new"].as_u64().unwrap_or_default();
        let matched = result["counts"]["matched"].as_u64().unwrap_or_default();
        let name = wc.chart().display_name(&AccountName::new(target));
        let source = result["source"]["format"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let summary = format!("import: {added} added, {matched} matched ({name}, {source})");
        let mut who = ops::who(state, caller, "import", &summary).await?;
        who.details["source"] = result["source"].clone();
        who.details["counts"] = result["counts"].clone();
        if let Some(filename) = body.filename {
            who.details["filename"] = json!(core::clean_text(&filename)
                .chars()
                .take(200)
                .collect::<String>());
        }
        let revision = ops::commit(state, changes, &who, wc.revision).await?;
        ops::remember(state, &wc, revision).await;
        result["head"] = json!(revision.to_string());
    }
    Ok(result)
}

/// Applies one statement to a working copy. The caller validates and commits the full copy.
pub(crate) fn apply_statement(
    wc: &mut WorkingCopy,
    target: &str,
    statement: Statement,
    options: &ImportOptions,
    simplefin_feed: Option<&SimplefinFeed>,
) -> Result<Value, Error> {
    let commodity = wc.commodity();
    let places = wc.places(&commodity);
    if let Some(reported) = statement.currency.filter(|reported| reported != &commodity) {
        return Err(ImportError::CurrencyMismatch {
            reported,
            commodity,
        }
        .into());
    }
    for row in &statement.rows {
        if row.amount.places() > places {
            return Err(ImportError::Field {
                location: row.import_id.clone(),
                field: "amount".into(),
            }
            .into());
        }
    }
    let chart = wc.chart();
    let planned = core::plan_rows(
        PlanInput {
            chart: &chart,
            transactions: &wc.book.transactions,
            target,
            commodity: &commodity,
            source: statement.source,
            simplefin_feed,
            since: options.since,
            decisions: &options.decisions,
        },
        statement.rows,
    )?;
    if options.require_unambiguous && planned.iter().any(|item| item.candidates.len() > 1) {
        return Err(crate::simplefin::problem(
            "simplefin_review",
            "Review bank sync in Settings to choose between matching transactions",
        ));
    }
    let classified = chart
        .get(target)
        .ok_or_else(|| ImportError::AccountKind(target.into()))?;
    let dry_run = options.dry_run.unwrap_or(true);
    let mut warnings = Vec::new();
    if statement.kind.is_some_and(|kind| kind != classified.kind) {
        warnings.push(json!({"code": "statement_kind", "message": "The statement and the selected account have different types"}));
    }
    let hint = statement
        .account_hint
        .as_ref()
        .map(|hint| format!("ofx:{hint}"));
    let tags = wc
        .book
        .accounts
        .get(target)
        .map_or("", |account| account.tags.as_str());
    let mut account_tags = kabacus_core::declared_tags(tags);
    if let Some(hint) = &hint {
        if account_tags.iter().any(|(key, value)| {
            key == "import-account" && value.starts_with("ofx:") && value != hint
        }) {
            warnings.push(json!({"code": "statement_account", "message": "This statement has a different account number from the previous import"}));
        }
    }
    let mut counts = [0; 4];
    let mut rows = Vec::new();
    let from = planned.iter().map(|p| p.row.date).min();
    let to = planned.iter().map(|p| p.row.date).max();
    for item in planned {
        let row = item.row;
        let (status, transaction) = match item.outcome {
            Outcome::New(tx) => {
                counts[0] += 1;
                if !wc.is_declared(UNCATEGORIZED) {
                    wc.declare(UNCATEGORIZED, "");
                }
                kabacus_core::check_description(&tx.description).map_err(ops::OpError::from)?;
                let change = wc.add(Transaction {
                    id: 0,
                    date: row.date,
                    status: Status::Cleared,
                    code: String::new(),
                    description: tx.description,
                    comment: String::new(),
                    tags: core::import_tags(target, &row.import_id)
                        .into_iter()
                        .chain(simplefin_feed.map(SimplefinFeed::tag))
                        .collect(),
                    postings: tx
                        .postings
                        .into_iter()
                        .map(|(account, money)| Posting {
                            account,
                            amount: Amount::new(&commodity, money),
                            status: Status::Unmarked,
                            kind: PostingKind::Real,
                            comment: String::new(),
                            tags: vec![],
                            cost: None,
                            balance_assertion: None,
                        })
                        .collect(),
                })?;
                ("new", change.id.filter(|_| !dry_run))
            }
            Outcome::Matched(id) => {
                counts[1] += 1;
                let index = wc.find(id).ok_or_else(|| Error::NotFound(id.to_string()))?;
                let mut tx = wc.tx(index).clone();
                tx.tags.extend(core::import_tags(target, &row.import_id));
                tx.tags.extend(simplefin_feed.map(SimplefinFeed::tag));
                tx.status = Status::Cleared;
                wc.replace(index, tx)?;
                ("matched", Some(id))
            }
            Outcome::Duplicate => {
                counts[2] += 1;
                if !options.include_duplicates {
                    continue;
                }
                ("duplicate", None)
            }
            Outcome::Skipped => {
                counts[3] += 1;
                ("skipped", None)
            }
        };
        let mut result = json!({"import_id": row.import_id, "date": row.date, "amount": row.amount.to_string(),
            "description": row.description, "outcome": status});
        if let Some(id) = transaction {
            result["transaction"] = json!(id);
        }
        if !item.candidates.is_empty() {
            result["candidates"] = json!(item.candidates);
        }
        rows.push(result);
    }
    if counts[0] + counts[1] != 0 {
        if let Some(hint) = hint {
            account_tags
                .retain(|(key, value)| key != "import-account" || !value.starts_with("ofx:"));
            account_tags.push(("import-account".into(), hint));
            let text = with_tags(tags_for(wc, target), &account_tags);
            wc.declare(target, &text);
        }
    } else {
        warnings.push(json!({"code": "unchanged", "message": "There are no new transactions or matches to write"}));
    }
    let page = search::page(
        rows,
        options.cursor.as_deref(),
        options.limit,
        search::DEFAULT_LIMIT,
    )?;
    let mut result = json!({"head": wc.head, "dry_run": dry_run, "account": target,
        "source": {"format": statement.source, "account_hint": statement.account_hint, "from": from, "to": to},
        "counts": {"new": counts[0], "matched": counts[1], "duplicate": counts[2], "skipped": counts[3], "error": 0},
        "rows": page.items, "next_cursor": page.next_cursor, "warnings": warnings});
    if let Some(reported) = statement.reported_balance {
        let balances = Balances::compute(&wc.book.transactions, &commodity, Some(reported.date));
        let actual = if classified.kind == AccountKind::Money {
            balances.inclusive(&classified.account)
        } else {
            balances.exclusive(&classified.account)
        }
        .money
        .rescaled(places);
        let reported_amount = reported.amount.rescaled(places);
        result["balance_check"] = json!({"date": reported.date, "reported": reported_amount.to_string(),
            "books": actual.to_string(), "difference": (actual - reported_amount).to_string()});
    }
    Ok(result)
}

fn tags_for<'a>(wc: &'a WorkingCopy, account: &str) -> &'a str {
    wc.book
        .accounts
        .get(account)
        .map_or("", |declaration| declaration.tags.as_str())
}
