// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bringing books in and taking them out: a journal tree read by the
//! engine into the (empty) ledger the server serves, and ledgers exported
//! and imported as `.kabacus` files. Implements the engine implementation
//! specification, sections 1.5, 1.8 and 3.4.

use crate::caller::{Caller, Permission as Role};
use crate::error::Error as ApiError;
use crate::service::{check_rewrite_gates, invalid, require};
use crate::state::Service;
use kabacus_ledger::{ImportError, ImportOutcome, ImportSource, MemorySource, Noted};
use kabacus_store::{BookStore, LedgerId};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How many of the report's entries a response lists.
const REPORT_ENTRIES: usize = 200;

/// What `POST /import` takes: the journal files by name, and which one is
/// the main journal (the others are reached by its `include`s).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct JournalImportBody {
    /// The main journal's name among `files`.
    pub main: String,
    /// Each journal's text, by its name (relative paths, as `include`
    /// names them).
    pub files: BTreeMap<String, String>,
    /// Defaults to false: read and check, then import.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportEntryDto {
    pub noted: String,
    pub file: String,
    pub line: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReportDto {
    /// How many of each thing import ignored or changed.
    pub counts: BTreeMap<String, usize>,
    /// The first entries, each with its file and line.
    pub entries: Vec<ReportEntryDto>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct JournalImportDto {
    pub head: String,
    pub dry_run: bool,
    pub transactions: usize,
    pub accounts: usize,
    pub commodities: usize,
    pub report: ImportReportDto,
}

pub fn report_dto(report: &kabacus_ledger::ImportReport) -> ImportReportDto {
    ImportReportDto {
        counts: report
            .counts()
            .into_iter()
            .map(|(k, n)| (k.as_str().to_string(), n))
            .collect(),
        entries: report
            .entries
            .iter()
            .take(REPORT_ENTRIES)
            .map(|e| ReportEntryDto {
                noted: e.noted.as_str().to_string(),
                file: e.file.clone(),
                line: e.line,
                detail: e.detail.clone(),
            })
            .collect(),
        truncated: report.entries.len() > REPORT_ENTRIES,
    }
}

/// An import that stopped, as the caller sees it.
pub fn import_error(e: ImportError) -> ApiError {
    match e {
        ImportError::Stopped { .. } => invalid("import", e.to_string()),
        ImportError::Invalid(failures) => ApiError::Invalid {
            rule: "import".to_string(),
            message: failures
                .iter()
                .map(|f| f.message())
                .collect::<Vec<_>>()
                .join("; "),
            failures,
        },
    }
}

/// Read `main` and what it includes from `src`.
pub fn read_journal(src: &dyn ImportSource, main: &Path) -> Result<ImportOutcome, ApiError> {
    kabacus_ledger::import(src, main).map_err(import_error)
}

/// Import a journal tree into the ledger this server serves, which must
/// be empty. Behind the rewrite gates.
pub async fn import_journal(
    state: &Service,
    principal: &dyn Caller,
    body: JournalImportBody,
) -> Result<JournalImportDto, ApiError> {
    check_rewrite_gates(state, principal)?;
    let root = PathBuf::from("/import");
    let mut files = BTreeMap::new();
    for (name, text) in body.files {
        let relative = Path::new(&name);
        let inside = !name.is_empty()
            && relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !inside {
            return Err(invalid(
                "files",
                format!("{name:?} isn't a file name inside the books"),
            ));
        }
        files.insert(root.join(relative), text);
    }
    let src = MemorySource(files);
    let outcome = read_journal(&src, &root.join(&body.main))?;
    let books = state.books_snapshot().await.map_err(ApiError::Internal)?;
    if !books.book.transactions.is_empty()
        || !books.book.accounts.is_empty()
        || !books.book.commodities.is_empty()
    {
        return Err(invalid(
            "import",
            "books are only imported into an empty ledger; these books already hold entries"
                .to_string(),
        ));
    }
    let mut result = JournalImportDto {
        head: books.key.clone(),
        dry_run: body.dry_run,
        transactions: outcome.book.transactions.len(),
        accounts: outcome.book.accounts.len(),
        commodities: outcome.book.commodities.len(),
        report: report_dto(&outcome.report),
    };
    if body.dry_run {
        return Ok(result);
    }
    let who = crate::ops::who(
        state,
        principal,
        "import",
        &format!("import: {}", body.main),
    )
    .await
    .map_err(ApiError::from)?;
    let revision = state
        .store
        .import(state.ledger, &outcome.book, &who)
        .await
        .map_err(|e| match e {
            kabacus_store::StoreError::Conflict { expected, current } => ApiError::Conflict {
                expected: expected.to_string(),
                current: current.to_string(),
            },
            other => ApiError::Internal(other.into()),
        })?;
    result.head = revision.to_string();
    Ok(result)
}

/// The lines `kabacus-server import` prints about what it did.
pub fn summary(outcome: &ImportOutcome) -> Vec<String> {
    let mut lines = vec![format!(
        "{} transaction(s), {} account(s), {} commodit{}",
        outcome.book.transactions.len(),
        outcome.book.accounts.len(),
        outcome.book.commodities.len(),
        if outcome.book.commodities.len() == 1 {
            "y"
        } else {
            "ies"
        }
    )];
    for (noted, n) in outcome.report.counts() {
        lines.push(format!("{}: {n}", noted.as_str()));
    }
    for e in outcome.report.entries.iter().filter(|e| {
        matches!(
            e.noted,
            Noted::PlacesRaised | Noted::AssertionSkipped | Noted::PostingWithoutAmount
        )
    }) {
        lines.push(format!("{}:{}: {}", e.file, e.line, e.detail));
    }
    lines
}

/// `GET /ledgers/{id}/export`: the ledger as a `.kabacus` file.
pub async fn export_ledger(
    state: &Service,
    principal: &dyn Caller,
    ledger: LedgerId,
) -> Result<(String, Vec<u8>), ApiError> {
    require(principal, Role::Read)?;
    if ledger != state.ledger {
        return Err(ApiError::NotFound(format!("ledger {ledger}")));
    }
    let name = state
        .store
        .ledgers()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .into_iter()
        .find(|l| l.id == ledger)
        .map(|l| l.name)
        .unwrap_or_else(|| "books".to_string());
    let bytes = state
        .store
        .export_bytes(&[ledger])
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok((format!("{}.kabacus", file_stem(&name)), bytes))
}

/// A ledger's name as a file name.
fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim().to_string();
    if stem.is_empty() {
        "books".to_string()
    } else {
        stem
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FileImportDto {
    /// The new ledgers' ids.
    pub ledgers: Vec<LedgerId>,
}

/// `POST /ledgers/import`: every ledger in a `.kabacus` file, as new
/// ledgers owned by the caller. Behind the rewrite gates.
pub async fn import_file(
    state: &Service,
    principal: &dyn Caller,
    bytes: &[u8],
) -> Result<FileImportDto, ApiError> {
    check_rewrite_gates(state, principal)?;
    if bytes.len() > state.settings.import_max_bytes {
        return Err(ApiError::BatchTooLarge(format!(
            "the file is {} bytes; at most {} can be imported",
            bytes.len(),
            state.settings.import_max_bytes
        )));
    }
    let owner = state.user_of(principal).await.map_err(ApiError::Internal)?;
    let ledgers = state
        .store
        .import_bytes(bytes, owner)
        .await
        .map_err(|e| match e {
            kabacus_store::StoreError::File(reason) => invalid("kabacus_file", reason),
            other => ApiError::Internal(other.into()),
        })?;
    Ok(FileImportDto { ledgers })
}
