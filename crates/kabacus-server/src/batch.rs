// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The server's batch deadline around the shared writer.

use crate::{api::ApiError, auth::roles::Principal, state::AppState};
pub use kabacus_service::batch::*;
use serde_json::Value;

pub async fn batch(
    state: &AppState,
    principal: &Principal,
    body: BatchBody,
) -> Result<Value, ApiError> {
    match tokio::time::timeout(
        TIME_LIMIT,
        kabacus_service::batch::batch(state, principal, body),
    )
    .await
    {
        Ok(result) => result.map_err(Into::into),
        Err(_) => Err(ApiError::BatchTooLarge(format!(
            "the batch took longer than {} seconds; split it into smaller batches",
            TIME_LIMIT.as_secs()
        ))),
    }
}
