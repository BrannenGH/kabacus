// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Import and export of native `.kabacus` files.

use crate::caller::{Caller, Permission as Role};
use crate::error::Error as ApiError;
use crate::service::{check_rewrite_gates, invalid, require};
use crate::state::Service;
use kabacus_store::{BookStore, LedgerId};
use serde::Serialize;

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
